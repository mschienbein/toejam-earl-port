//! Authored Genesis programs drive the actual VDP IRQ lines and original CPU
//! RTE. These are protocol regressions, not retail-ROM or game-body proofs.
use super::*;
use crate::sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use genesis_core::GenesisEmulator;
use jgenesis_common::frontend::EmulatorTrait;
use m68000_emu::{BusInterface, ExceptionBoundary};

fn put(rom: &mut [u8], pc: u32, words: &[u16]) {
    for (index, word) in words.iter().enumerate() {
        let offset = pc as usize + 2 * index;
        rom[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
    }
}
fn machine(nested: bool) -> GenesisEmulator {
    let mut rom = vec![0; 0x100000];
    rom[..4].copy_from_slice(&SP.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    rom[28 * 4..29 * 4].copy_from_slice(&0x200_u32.to_be_bytes());
    rom[30 * 4..31 * 4].copy_from_slice(&0x300_u32.to_be_bytes());
    put(
        &mut rom,
        0x100,
        &[0x4eb9, (ROOT >> 16) as u16, ROOT as u16, 0x60fe],
    );
    let target = child();
    put(
        &mut rom,
        ROOT,
        &[0x4eb9, (target >> 16) as u16, target as u16, 0x4e75],
    );
    // A short authored child loops across a real HBlank, then returns normally.
    put(
        &mut rom,
        target,
        &[0x303c, 0x1000, 0x4e71, 0x51c8, 0xfffc, 0x4e75],
    );
    if nested {
        // IRQ4 disables further HBlank IRQs and spends enough real CPU time in
        // DBF D1 for the VDP's higher-priority VBlank IRQ6 to preempt it.
        put(
            &mut rom,
            0x200,
            &[
                0x33fc, 0x8000, 0x00c0, 0x0004, 0x323c, 0x4000, 0x51c9, 0xfffe, 0x4e73,
            ],
        );
    } else {
        put(&mut rom, 0x200, &[0x33fc, 0x8000, 0x00c0, 0x0004, 0x4e73]);
    }
    put(&mut rom, 0x300, &[0x5283, 0x4e73]);
    let mut machine = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    machine.hybrid_parts().0.set_status_register(0x2000);
    for value in [0x8010, if nested { 0x8120 } else { 0x8100 }, 0x8a00] {
        machine.hybrid_bus_mut().write_word(0xc00004, value);
        machine.hybrid_bus_mut().apply_pending_writes();
    }
    machine.hybrid_parts().0.hybrid_enable_instruction_counts();
    machine
}
fn tick(machine: &mut GenesisEmulator, audio: &mut HeadlessAudio) {
    machine
        .tick(
            &mut HeadlessOutput,
            audio,
            &mut Inputs(GenesisInputs::default()),
            &mut NoCartridgeSaves,
        )
        .unwrap();
}
fn return_pc(machine: &GenesisEmulator, sp: u32) -> u32 {
    let memory = &machine.hybrid_bus().memory;
    (u32::from(memory.read_main_ram_word(sp)) << 16)
        | u32::from(memory.read_main_ram_word(sp.wrapping_add(2)))
}
fn instruction_total(machine: &GenesisEmulator) -> u64 {
    let counts = machine.hybrid_cpu().hybrid_instruction_counts().unwrap();
    counts.main_thread + counts.exception_handler
}
fn actual_irq_case(nested: bool) {
    let mut machine = machine(nested);
    let mut audio = HeadlessAudio::default();
    for _ in 0..1024 {
        if machine.hybrid_cpu().pc() == ROOT {
            break;
        }
        tick(&mut machine, &mut audio);
    }
    let entry = InvocationPoint::capture(machine.hybrid_cpu());
    assert_eq!(
        (entry.pc, entry.sp, entry.exception_depth),
        (ROOT, SP - 4, 0)
    );
    assert_eq!(
        return_pc(&machine, entry.sp),
        0x106,
        "the actual bootstrap JSR supplies the root frame"
    );
    let mut tracker = InvocationTracker::rooted(
        Revision::Rev02,
        ids::NO_OP,
        ROOT,
        entry.sp,
        entry,
        Some(return_pc(&machine, entry.sp)),
        false,
    );
    let root_witness = tracker.current_witness().unwrap();
    machine.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
    machine.hybrid_bus_mut().hybrid_exception_boundaries = Some(Vec::new());
    let mut boundaries = Vec::new();
    let mut child_id = None;
    let mut raw_child = None;
    let mut returned_child = false;
    let mut returned_root = false;
    for _ in 0..200_000 {
        let before = InvocationPoint::capture(machine.hybrid_cpu());
        let ready = machine.hybrid_bus().cycles.m68k_wait_cpu_cycles == 0
            && machine
                .hybrid_cpu()
                .hybrid_instruction_ready(machine.hybrid_bus());
        let total = instruction_total(&machine);
        tracker.begin_actual_step(before, machine.hybrid_cpu().next_opcode(), ready);
        tick(&mut machine, &mut audio);
        let after = InvocationPoint::capture(machine.hybrid_cpu());
        let instruction = (instruction_total(&machine) > total).then(|| {
            machine
                .hybrid_cpu()
                .hybrid_instruction_counts()
                .unwrap()
                .last_instruction
                .unwrap()
        });
        tracker.observe_point(after);
        {
            let bus = machine.hybrid_bus_mut();
            let payloads = bus.hybrid_exception_boundaries.as_deref().unwrap();
            for access in bus.hybrid_cpu_accesses.as_mut().unwrap().drain(..) {
                tracker.observe_actual_access(access);
                if let HybridAccess::ExceptionBoundary { index } = access {
                    // Feed the exact indexed provider payload, with no depth
                    // adjustment, reconstruction or alternative success path.
                    let boundary = payloads.get(index as usize).unwrap().boundary;
                    let update = tracker.observe_exception_boundary(boundary);
                    assert_eq!(
                        update.invalidated,
                        Some(Invalidation::ExceptionOrBank),
                        "{boundary:?}"
                    );
                    match boundary {
                        ExceptionBoundary::Entered { depth, .. } => {
                            let id = update.exception_entered.unwrap();
                            let record = tracker.exception_record(id).unwrap();
                            assert_eq!(record.depth, depth);
                            assert_eq!(
                                record.call_parent, child_id,
                                "IRQ really interrupted the live child call"
                            );
                            assert_eq!(tracker.exception_active.len(), depth as usize);
                        }
                        ExceptionBoundary::Returned { depth } => {
                            let id = update
                                .exception_returned
                                .expect("a genuine CPU RTE must close its exact entered context");
                            assert_eq!(
                                tracker.exception_record(id).unwrap().depth.checked_sub(1),
                                Some(depth)
                            );
                            assert_eq!(tracker.exception_active.len(), depth as usize);
                            assert_eq!(
                                tracker.current().unwrap().owner(),
                                Some(ids::GET_RANDOM_NUMBER)
                            );
                        }
                    }
                    assert!(
                        tracker.current_witness().is_none(),
                        "IRQ/RTE never rearms parent reads"
                    );
                    assert!(tracker.step_witness().is_none());
                    boundaries.push(boundary);
                }
            }
            bus.hybrid_exception_boundaries.as_mut().unwrap().clear();
        }
        let frame = instruction
            .filter(|instruction| successful_call_instruction(*instruction))
            .map(|_| return_pc(&machine, after.sp));
        let update = tracker.finish_actual_step(instruction, after, frame);
        if let Some(id) = update.entered {
            assert_eq!(
                tracker.record(id).unwrap().owner(),
                Some(ids::GET_RANDOM_NUMBER)
            );
            assert!(
                tracker.current_witness().is_some(),
                "real child permission exists before the first IRQ"
            );
            child_id = Some(id);
            raw_child = Some(tracker.actual_call(id).unwrap());
        }
        if let Some(id) = update.returned {
            if Some(id) == child_id {
                returned_child = true;
                assert!(
                    tracker.current_witness().is_none(),
                    "RTS after RTE must not rearm the suspended root"
                );
            } else {
                returned_root = true;
                break;
            }
        }
    }
    assert!(
        returned_child && returned_root,
        "actual IRQ/call source did not finish"
    );
    assert!(tracker.exception_active.is_empty());
    assert!(tracker.current().is_none());
    assert!(tracker.current_witness().is_none());
    let raw = tracker.resolve_actual(raw_child.unwrap()).unwrap();
    assert_eq!(
        (raw.owner(), raw.entry_sp(), raw.return_site()),
        (Some(ids::GET_RANDOM_NUMBER), Some(SP - 8), Some(ROOT + 6))
    );
    assert!(
        tracker.resolve(root_witness).is_some(),
        "read history remains historical evidence only"
    );
    let expected = if nested {
        vec![
            ExceptionBoundary::Entered {
                vector: 28,
                handler: 0x200,
                depth: 1,
            },
            ExceptionBoundary::Entered {
                vector: 30,
                handler: 0x300,
                depth: 2,
            },
            ExceptionBoundary::Returned { depth: 1 },
            ExceptionBoundary::Returned { depth: 0 },
        ]
    } else {
        vec![
            ExceptionBoundary::Entered {
                vector: 28,
                handler: 0x200,
                depth: 1,
            },
            ExceptionBoundary::Returned { depth: 0 },
        ]
    };
    assert_eq!(boundaries, expected);
}

#[test]
fn real_cpu_irq_inside_call_pairs_post_rte_depth_without_rearming_reads() {
    actual_irq_case(false);
}
#[test]
fn real_cpu_nested_irq_rtes_close_inner_then_outer_using_post_return_depth() {
    actual_irq_case(true);
}
#[test]
fn mismatched_skipped_duplicate_and_underflowed_return_depths_fail_closed() {
    for returned in [1, 2, u32::MAX] {
        let mut tracker = rooted();
        let outer = tracker
            .observe_exception_boundary(ExceptionBoundary::Entered {
                vector: 28,
                handler: 0x200,
                depth: 1,
            })
            .exception_entered
            .unwrap();
        let update =
            tracker.observe_exception_boundary(ExceptionBoundary::Returned { depth: returned });
        assert_eq!(update.invalidated, Some(Invalidation::Lifecycle));
        assert!(update.exception_returned.is_none());
        assert_eq!(tracker.exception_active, vec![outer]);
        assert!(tracker.current_witness().is_none());
    }
    let mut tracker = rooted();
    tracker.observe_exception_boundary(ExceptionBoundary::Entered {
        vector: 28,
        handler: 0x200,
        depth: 1,
    });
    let inner = tracker
        .observe_exception_boundary(ExceptionBoundary::Entered {
            vector: 30,
            handler: 0x300,
            depth: 2,
        })
        .exception_entered
        .unwrap();
    let skipped = tracker.observe_exception_boundary(ExceptionBoundary::Returned { depth: 0 });
    assert_eq!(skipped.invalidated, Some(Invalidation::Lifecycle));
    assert!(skipped.exception_returned.is_none());
    assert_eq!(tracker.exception_active.last(), Some(&inner));
    assert!(tracker.current_witness().is_none());

    let mut tracker = rooted();
    let entry = tracker
        .observe_exception_boundary(ExceptionBoundary::Entered {
            vector: 28,
            handler: 0x200,
            depth: 1,
        })
        .exception_entered
        .unwrap();
    tracker.exception_records[entry.index].depth = 0; // authored malformed-entry control
    let underflow =
        tracker.observe_exception_boundary(ExceptionBoundary::Returned { depth: u32::MAX });
    assert_eq!(underflow.invalidated, Some(Invalidation::Lifecycle));
    assert!(underflow.exception_returned.is_none());
    assert_eq!(tracker.exception_active, vec![entry]);

    let mut tracker = rooted();
    tracker.observe_exception_boundary(ExceptionBoundary::Entered {
        vector: 28,
        handler: 0x200,
        depth: 1,
    });
    assert!(
        tracker
            .observe_exception_boundary(ExceptionBoundary::Returned { depth: 0 })
            .exception_returned
            .is_some()
    );
    let duplicate = tracker.observe_exception_boundary(ExceptionBoundary::Returned { depth: 0 });
    assert_eq!(duplicate.invalidated, Some(Invalidation::Lifecycle));
    assert!(duplicate.exception_returned.is_none());
    assert!(tracker.current_witness().is_none());
}
