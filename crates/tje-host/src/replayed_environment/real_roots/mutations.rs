//! Private-cartridge mutants at real source calls. No machine fixture, fake
//! readiness or canonical-body promotion is used. The old frame600 authored
//! comparator fixture remains separately HELD, never retroactively passed.
use super::*;
use tje_game::{
    Context,
    execution::{RamWidth, WaitComparison, WaitCondition, WaitRequest},
};
use tje_mem::ram;

fn wrong_wait_relation(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    // Same readable source reads, count loop and interrupted D1 as the owner.
    // The sole mutation is Equal instead of NotEqual in the dedicated wait.
    let frame = context
        .frame_state()
        .ok_or(tje_game::call_frame::FrameError::Unavailable)?;
    let entry = ids::WAIT_FOR_NEXT_V_BLANK;
    if !frame.is_prepared_for(entry) || context.stack_pointer()? != frame.entry_sp {
        return Err(tje_game::call_frame::FrameError::PreparedEntryRequired(entry).into());
    }
    let mut count = frame.argument_word(0).read(context);
    let revision = context.revision();
    if ram::g_mode2_reg_softcopy(revision).read(context) & 0x40 != 0 {
        while count > 0 {
            let counter = ram::g_frame_counter(revision);
            let previous = counter.read(context);
            let d1 = context.data_register(1);
            context.set_data_register(1, (d1 & 0xffff0000) | u32::from(previous));
            context.wait_until(WaitRequest::new(WaitCondition::Ram {
                address: counter.address(),
                width: RamWidth::Word,
                mask: 0xffff,
                value: u32::from(previous),
                comparison: WaitComparison::Equal,
            }))?;
            count -= 1;
        }
    }
    Ok(None)
}
static MUTANT: LazyLock<Vec<FramedTranslation>> = LazyLock::new(|| {
    FRAMED
        .iter()
        .copied()
        .map(|mut entry| {
            if entry.id == ids::WAIT_FOR_NEXT_V_BLANK {
                entry.function = wrong_wait_relation;
            }
            entry
        })
        .collect()
});

#[test]
#[ignore = "requires both private ROMs; genuine frame492 correct and wrong-relation actual-call checks"]
fn genuine_wait_492_wrong_relation_rejects_at_the_authenticated_poll_both_revisions() {
    let id = ids::WAIT_FOR_NEXT_V_BLANK;
    let script = Arc::new(ScenarioInputs {
        source: "crates/tje-host/examples/start-game.json".into(),
        frames: 494,
        timeline: Timeline::Holds(
            InputScript::parse(include_str!("../../../examples/start-game.json")).unwrap(),
        ),
    });
    for revision in [Revision::Rev02, Revision::Rev00] {
        for mutant in [false, true] {
            let registry = HookRegistry::proof_replayed_environment(
                revision,
                &VALUES,
                if mutant { &MUTANT } else { &FRAMED },
                &[id],
            )
            .unwrap();
            let mut host = Host::new(&rom(revision), Mode::Reference, registry).unwrap();
            host.set_wait_input_poller(script.clone());
            let mut cache = FrameInputs {
                frame: None,
                input: Inputs(Default::default()),
            };
            while host.frame_count() < 493
                && (host.frame_count() < 492
                    || target(&host) != Some(id.metadata().address(revision)))
            {
                step(&mut host, &script, &mut cache).unwrap();
            }
            assert_eq!(
                host.frame_count(),
                492,
                "the exact real caller must be reached"
            );
            assert_eq!(target(&host), Some(id.metadata().address(revision)));
            let call_pc = host.cpu().pc();
            let prefix_clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
            host.mode = Mode::Check;
            let result = (|| -> crate::Result<()> {
                loop {
                    step(&mut host, &script, &mut cache)?;
                    let report = host.checks.replayed_environment.as_ref().unwrap();
                    if report.by_function[id.metadata().inventory_id].checked != 0 {
                        break;
                    }
                    assert!(
                        host.frame_count() < 494,
                        "actual original call must remain in its fixed window"
                    );
                }
                Ok(())
            })();
            let finalization = host.finalize_replayed_campaign();
            let report = host.checks.replayed_environment.as_ref().unwrap();
            let root = &report.by_function[id.metadata().inventory_id];
            assert_eq!(root.entered, 1);
            assert_eq!(root.reference_completed, 1);
            assert_eq!(root.checked, 1);
            if mutant {
                assert!(
                    matches!(&result, Err(crate::HostError::Mismatch { .. })),
                    "{revision:?}: {result:?}"
                );
                assert!(
                    result
                        .as_ref()
                        .unwrap_err()
                        .to_string()
                        .contains("wrong wait predicate"),
                    "wrong relation must fail at the authenticated original poll: {result:?}"
                );
                assert!(finalization.is_err());
                assert_eq!((root.own_matches, root.passed, root.failed), (0, 0, 1));
                assert_eq!(report.mismatches, 1);
                assert_eq!(report.unsupported, 0);
                assert_eq!(report.failures.len(), 1);
                assert_eq!(root.first_failure_index, Some(0));
            } else {
                result.as_ref().unwrap();
                finalization.as_ref().unwrap();
                assert_eq!((root.own_matches, root.passed, root.failed), (1, 1, 0));
                assert!(report.complete && report.finalized);
                assert_eq!(
                    (
                        report.open_segments,
                        report.unresolved_obligations,
                        report.mismatches
                    ),
                    (0, 0, 0)
                );
            }
            println!(
                "REPLAY_GENUINE_WAIT_MUTATION {}",
                serde_json::json!({
                    "revision":revision.name(),"mutant":mutant,"inventory_id":id.metadata().inventory_id,
                    "input_session":script.source,"entry_frame":492,"return_frame":host.frame_count(),
                    "activation_original_pc":call_pc,"activation_phase":"check_before_actual_jsr_step",
                    "unchecked_prefix_frames":492,"unchecked_prefix_master_clock":prefix_clock,
                    "prefix_scope":"not startup or composition proof",
                    "mutation":"Equal instead of NotEqual after identical source initial D1 materialization",
                    "result":result.as_ref().err().map(ToString::to_string),
                    "finalization_error":finalization.as_ref().err().map(ToString::to_string),
                    "root":root,"complete":report.complete,"finalized":report.finalized,
                    "mismatches":report.mismatches,"unsupported":report.unsupported,
                    "open_segments":report.open_segments,"unresolved_obligations":report.unresolved_obligations,
                    "failures":report.failures,
                })
            );
        }
    }
}
