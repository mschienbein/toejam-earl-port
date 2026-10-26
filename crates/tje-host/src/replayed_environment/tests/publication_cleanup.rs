//! Real authored root captures; only the private publication boundary faults.
use super::*;
use crate::replayed_environment::host::publish_replayed_state;
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
};

fn collect_completed(host: &mut Host) {
    let mut state = host.replayed_state.take().unwrap();
    let registry = host.registry.clone();
    for _ in 0..10000 {
        state
            .before_tick(
                &mut host.emulator,
                host.frame,
                host.checks.replayed_environment.as_mut().unwrap(),
            )
            .unwrap();
        let effect = host.tick(&mut Inputs(GenesisInputs::default())).unwrap();
        let rendered = effect == TickEffect::FrameRendered;
        state.after_tick(
            &mut host.emulator,
            &registry,
            &std::collections::BTreeSet::new(),
            rendered,
            GenesisInputs::default(),
            host.checks.replayed_environment.as_mut().unwrap(),
        );
        if rendered {
            host.frame += 1;
        }
        if state.has_pending() {
            assert!(
                state.take_error().is_none(),
                "capture must succeed; only publication faults"
            );
            host.replayed_state = Some(state);
            return;
        }
    }
    panic!("authored actual root did not complete");
}
fn captured(framed: &'static [FramedTranslation], twice: bool) -> Host {
    let revision = Revision::Rev02;
    let mut main = vec![0x46fc, 0x2700];
    main.extend(jsr(ROOT, revision));
    if twice {
        main.extend(jsr(ROOT, revision));
    }
    main.push(0x60fe);
    let mut host = host(
        revision,
        &[],
        framed,
        &[ROOT],
        &main,
        &[(ROOT, literal_code())],
    );
    host.context().write_u8(VALUE, 8);
    collect_completed(&mut host);
    host
}
fn encoded(host: &Host) -> Vec<u8> {
    bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
}
fn fatal(host: &Host) {
    let state = host
        .replayed_state
        .as_ref()
        .expect("private state restored before every exit");
    assert!(state.failed);
    assert!(state.infrastructure_failure().is_some());
}
fn no_revival(host: &mut Host, prior_passes: u64) {
    fatal(host);
    let before = encoded(host);
    assert!(
        step(host).is_err(),
        "ContinueOriginal never turns infrastructure loss into unchecked play"
    );
    assert_eq!(
        encoded(host),
        before,
        "later rejection precedes real guest/device progress"
    );
    assert!(host.finalize_replayed_campaign().is_err());
    let report = host.checks.replayed_environment.as_ref().unwrap();
    assert!(report.finalized && !report.complete);
    assert_eq!(report.completed_proofs, prior_passes);
    assert_eq!(
        report.by_function[ROOT.metadata().inventory_id].passed,
        prior_passes
    );
}

#[test]
fn publication_result_failure_is_fatal_in_both_policies_and_never_replaces_earlier_root_result() {
    for policy in [
        CheckFailurePolicy::FailFast,
        CheckFailurePolicy::ContinueOriginal,
    ] {
        for (body, mismatch) in [(LITERAL, false), (WRONG, true)] {
            let mut host = captured(body, false);
            host.check_failure_policy = policy;
            let before = encoded(&host);
            let mut removed = None;
            let error = host
                .flush_replays_with_publisher(|state, checks| {
                    removed = checks.replayed_environment.take();
                    // The SAME production missing-report branch, not an invented
                    // checker/body result or a fake canonical positive.
                    publish_replayed_state(state, checks)
                })
                .unwrap_err();
            if mismatch {
                assert!(
                    matches!(&error, crate::HostError::Mismatch { .. }),
                    "{error}"
                );
                let report = removed.as_ref().unwrap();
                assert_eq!(
                    report.failures[0].check.as_ref().unwrap().detail,
                    error.to_string(),
                    "exact original first Result error survives the later cleanup error"
                );
            } else {
                assert!(
                    matches!(&error, crate::HostError::Metadata(detail)
                    if detail == "replay output report was removed"),
                    "{error}"
                );
            }
            fatal(&host);
            assert!(
                host.replayed_state
                    .as_ref()
                    .unwrap()
                    .infrastructure_failure()
                    .unwrap()
                    .detail
                    .contains("report was removed")
            );
            assert_eq!(encoded(&host), before);
            host.checks.replayed_environment = removed;
            no_revival(&mut host, 0);
        }
    }
}

#[derive(Debug)]
struct PublicationPanic(u64);
#[test]
fn unrelated_publication_panic_survives_a_prior_root_result_and_withdraws_only_uncommitted_credit()
{
    for policy in [
        CheckFailurePolicy::FailFast,
        CheckFailurePolicy::ContinueOriginal,
    ] {
        for (body, mismatch) in [(LITERAL, false), (WRONG, true)] {
            let mut host = captured(body, false);
            host.check_failure_policy = policy;
            let before = encoded(&host);
            let marker = Box::new(PublicationPanic(0x1234_5678));
            let identity = std::ptr::from_ref(marker.as_ref());
            let caught = catch_unwind(AssertUnwindSafe(|| {
                host.flush_replays_with_publisher(|state, checks| {
                    publish_replayed_state(state, checks)?;
                    // Fault AFTER real production publication, including consuming
                    // its pending set: finalization must still not revive this pass.
                    resume_unwind(marker)
                })
            }));
            let payload = match caught {
                Err(payload) => payload,
                Ok(_) => panic!("publication panic was swallowed"),
            };
            let Ok(marker) = payload.downcast::<PublicationPanic>() else {
                panic!("publication payload type changed");
            };
            assert!(std::ptr::eq(marker.as_ref(), identity));
            assert_eq!(marker.0, 0x1234_5678);
            assert_eq!(encoded(&host), before);
            if mismatch {
                assert_eq!(
                    host.checks.replayed_environment.as_ref().unwrap().failures[0].verdict,
                    Verdict::Mismatch
                );
            }
            no_revival(&mut host, 0);
        }
    }
}

#[derive(Debug)]
struct ExecutionPanic(u64);
thread_local! { static EXECUTION: RefCell<Option<Box<ExecutionPanic>>> = const { RefCell::new(None) }; }
fn panic_body(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let _ = context.read_u8(VALUE);
    resume_unwind(EXECUTION.with(|slot| slot.borrow_mut().take().unwrap()))
}
static PANIC: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: panic_body,
}];
#[test]
fn original_owned_execution_payload_wins_over_an_unrelated_publisher_payload_after_all_cleanup() {
    for policy in [
        CheckFailurePolicy::FailFast,
        CheckFailurePolicy::ContinueOriginal,
    ] {
        let mut host = captured(PANIC, false);
        host.check_failure_policy = policy;
        let before = encoded(&host);
        let marker = Box::new(ExecutionPanic(0x8877_6655));
        let identity = std::ptr::from_ref(marker.as_ref());
        EXECUTION.with(|slot| *slot.borrow_mut() = Some(marker));
        let caught = catch_unwind(AssertUnwindSafe(|| {
            host.flush_replays_with_publisher(|state, checks| {
                publish_replayed_state(state, checks)?;
                resume_unwind(Box::new(PublicationPanic(0x1122)))
            })
        }));
        let payload = match caught {
            Err(payload) => payload,
            Ok(_) => panic!("execution payload was swallowed"),
        };
        let Ok(marker) = payload.downcast::<ExecutionPanic>() else {
            panic!("a later cleanup panic replaced execution payload");
        };
        assert!(std::ptr::eq(marker.as_ref(), identity));
        assert_eq!(marker.0, 0x8877_6655);
        assert_eq!(encoded(&host), before);
        no_revival(&mut host, 0);
    }
}

#[test]
fn failed_publication_keeps_previous_published_cases_but_never_revives_the_new_case() {
    let mut host = captured(LITERAL, true);
    host.flush_completed_replays().unwrap();
    assert_eq!(
        host.checks
            .replayed_environment
            .as_ref()
            .unwrap()
            .completed_proofs,
        1
    );
    assert_eq!(host.replayed_state.as_ref().unwrap().proofs.len(), 1);
    collect_completed(&mut host);
    let before = encoded(&host);
    let mut removed = None;
    assert!(
        host.flush_replays_with_publisher(|state, checks| {
            removed = checks.replayed_environment.take();
            publish_replayed_state(state, checks)
        })
        .is_err()
    );
    assert_eq!(encoded(&host), before);
    assert_eq!(
        host.replayed_state.as_ref().unwrap().proofs.len(),
        2,
        "own-match and prior good history remain, never deleted or reset"
    );
    host.checks.replayed_environment = removed;
    no_revival(&mut host, 1);
}
