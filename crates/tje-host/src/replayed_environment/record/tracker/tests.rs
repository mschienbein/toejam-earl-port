//! Consumer binding controls over actual authored JSR/RTS captures. These do
//! not forge the shared opaque IDs or mint any body/read-permission proof.
use super::*;
use crate::replayed_environment::tests::causal_inputs;
use tje_game::ids;

#[test]
fn foreign_capture_tokens_and_diagnostic_ids_cannot_replace_the_actual_call_seal() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (host, original, tracker) =
            causal_inputs::capture_with_tracker(revision, vec![0x4e75], vec![]);
        let (_, foreign, _) = causal_inputs::capture_with_tracker(revision, vec![0x4e75], vec![]);
        tracker.validate(original.entry, 0).unwrap();
        let EntryEvidence::Call { raw, actual } = original.entry.evidence else {
            panic!("real root must have a shared actual-call seal")
        };
        let EntryEvidence::Call {
            raw: foreign_raw,
            actual: foreign_actual,
        } = foreign.entry.evidence
        else {
            panic!("foreign real root must have its own shared seal")
        };
        assert!(tracker.shared.resolve_actual(actual).is_some());
        assert!(
            tracker.shared.current_witness().is_none(),
            "historical identity never rearms a read permission"
        );
        let cpu = host.cpu().hybrid_state_words();
        let clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        for evidence in [
            foreign.entry.evidence,
            EntryEvidence::Call {
                raw,
                actual: foreign_actual,
            },
            EntryEvidence::Call {
                raw: foreign_raw,
                actual,
            },
        ] {
            let mut bad = original.entry;
            bad.evidence = evidence;
            assert!(tracker.validate(bad, 0).is_err());
        }
        let mut bad = original.entry;
        bad.identity = foreign.entry.identity;
        assert!(
            tracker.validate(bad, 0).is_err(),
            "a diagnostic trajectory is not a call source"
        );
        assert_eq!(host.cpu().hybrid_state_words(), cpu);
        assert_eq!(host.emulator.hybrid_bus().cycles.m68k_mclk_cycles, clock);
        tracker.validate(original.entry, 0).unwrap();
    }
}

#[test]
fn actual_seal_checks_owner_target_frame_bank_depth_source_site_and_parent() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (_, original, tracker) =
            causal_inputs::capture_with_tracker(revision, vec![0x4e75], vec![]);
        let EntryKind::Call {
            caller_pc,
            opcode,
            return_pc,
        } = original.entry.kind
        else {
            panic!("actual JSR root")
        };
        for mutation in 0..9 {
            let mut bad = original.entry;
            match mutation {
                0 => bad.function = Some(ids::PROCESS_INPUT),
                1 => bad.boundary.cpu.words[17] ^= 2,
                2 => bad.boundary.cpu.words[16] ^= 4,
                3 => bad.boundary.cpu.words[18] ^= 0x2000,
                4 => bad.boundary.cpu.exception_depth += 1,
                5 => {
                    bad.kind = EntryKind::Call {
                        caller_pc: caller_pc + 2,
                        opcode,
                        return_pc,
                    }
                }
                6 => {
                    bad.kind = EntryKind::Call {
                        caller_pc,
                        opcode: opcode ^ 1,
                        return_pc,
                    }
                }
                7 => {
                    bad.kind = EntryKind::Call {
                        caller_pc,
                        opcode,
                        return_pc: return_pc + 2,
                    }
                }
                8 => bad.parent = Some(original.entry.identity),
                _ => unreachable!(),
            }
            assert!(
                tracker.validate(bad, 0).is_err(),
                "unbound actual-call field {mutation}"
            );
        }
        let mut bad = original.entry;
        bad.kind = EntryKind::Interrupt { vector: 30 };
        assert!(
            tracker.validate(bad, 0).is_err(),
            "an actual call cannot be reclassified as an IRQ"
        );
        tracker.validate(original.entry, 0).unwrap();
    }
}
