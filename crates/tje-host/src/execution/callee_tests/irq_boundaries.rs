//! Real authored CPU/VDP boundaries, never injected IRQs or cartridge snapshots.
use super::*;

const HANDLER: u32 = 0x400;
const CALIBRATION: u32 = 0x600;
const OUTSIDE_RETURN: u32 = 0x106;
const CAPPED: &[tje_game::FunctionId] = &[
    ids::MAP_ASSIGN_TRANSITION_CHAIN,
    ids::EARTHLING_RETAIN_LEVEL_BANK,
    ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
    ids::MOVE_DATA_TO_VRAM,
];

fn machine(
    revision: Revision,
    function: tje_game::FunctionId,
    root: u32,
    body: &[u16],
    handler_calls_child: bool,
) -> GenesisEmulator {
    let target = function.metadata().address(revision);
    let mut rom = vec![0; (target as usize + 16).next_power_of_two().max(0x800)];
    rom[..4].copy_from_slice(&crate::catalogue::STACK_TOP.to_be_bytes());
    rom[4..8].copy_from_slice(&CALIBRATION.to_be_bytes());
    for vector in [28, 30, 32] {
        rom[vector * 4..vector * 4 + 4].copy_from_slice(&HANDLER.to_be_bytes());
    }
    let handler = if handler_calls_child {
        vec![0x4eb9, (target >> 16) as u16, target as u16, 0x4e73]
    } else {
        vec![0x4e73]
    };
    for (address, words) in [
        (CALIBRATION, &[0x4e71, 0x60fc][..]), // NOP; BRA.S calibration
        (HANDLER, handler.as_slice()),
        (root, body),
        (target, &[0x4e75][..]),
        (PARK_PC, &[0x60fe, 0x4e71][..]),
    ] {
        for (index, word) in words.iter().enumerate() {
            let offset = address as usize + 2 * index;
            rom[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let mut emulator = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    emulator.hybrid_parts().0.set_status_register(0x2700);
    let bus = emulator.hybrid_bus_mut();
    bus.write_word(0xc00004, 0x8010); // Enable real H interrupts.
    bus.write_word(0xc00004, 0x8a00); // First real H-counter expiry.
    bus.apply_pending_writes();
    emulator
}

fn physical_tick(emulator: &mut GenesisEmulator, audio: &mut HeadlessAudio) {
    emulator
        .tick(
            &mut HeadlessOutput,
            audio,
            &mut Inputs(Default::default()),
            &mut NoCartridgeSaves,
        )
        .unwrap();
}

fn before_hint(
    revision: Revision,
    function: tje_game::FunctionId,
    root: u32,
    body: &[u16],
    handler_calls_child: bool,
) -> GenesisEmulator {
    // First observe which actual masked NOP/BRA tick raises HINT, then replay
    // exactly its preceding real boundary on a fresh authored machine. JSR/BSR
    // and RTS take longer than either calibration opcode and cross that same
    // real device event. No pending bit or raster/CPU state is reconstructed.
    let mut probe = machine(revision, function, root, body, handler_calls_child);
    let mut audio = HeadlessAudio::default();
    let mut ticks = 0;
    while probe.hybrid_bus().interrupt_level() != 4 {
        assert!(
            ticks < 1000,
            "authored reference positioning did not reach HINT"
        );
        physical_tick(&mut probe, &mut audio);
        ticks += 1;
    }
    assert!(ticks > 0);
    let mut emulator = machine(revision, function, root, body, handler_calls_child);
    audio.samples.clear();
    for _ in 1..ticks {
        physical_tick(&mut emulator, &mut audio);
    }
    assert_eq!(emulator.hybrid_bus().interrupt_level(), 0);
    emulator
}

fn prepare_root(emulator: &mut GenesisEmulator, pc: u32, return_pc: u32, sr: u16) {
    // Ordinary authored fixture entry only; all measured child JSR/BSR frames
    // and every interrupt frame below are produced by the actual CPU.
    let sp = crate::catalogue::STACK_TOP - 4;
    let bus = emulator.hybrid_bus_mut();
    bus.write_word(sp, (return_pc >> 16) as u16);
    bus.write_word(sp + 2, return_pc as u16);
    bus.apply_pending_writes();
    let (cpu, bus) = emulator.hybrid_parts();
    cpu.set_status_register(sr);
    crate::abi::set_sp(cpu, sp);
    cpu.set_pc(pc, bus);
}

fn accept_real_hint_at_child(emulator: &mut GenesisEmulator, target: u32) -> u64 {
    let mut audio = HeadlessAudio::default();
    let mut ticks = 0;
    while emulator.hybrid_cpu().hybrid_exception_depth() == 0 {
        assert_eq!(emulator.hybrid_cpu().pc(), target);
        assert!(ticks < 10);
        physical_tick(emulator, &mut audio);
        ticks += 1;
    }
    assert_eq!(emulator.hybrid_cpu().pc(), HANDLER);
    assert_eq!(
        emulator.hybrid_cpu().hybrid_last_exception_vector(),
        Some(28)
    );
    assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 1);
    ticks
}

#[test]
fn own_isr_rte_to_capped_interrupted_entry_does_not_meter_the_outside_caller() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &function in CAPPED {
            let target = function.metadata().address(revision);
            let root = target - 32;
            let body = [0x4eb9, (target >> 16) as u16, target as u16, 0x4e75];
            let mut emulator = before_hint(revision, function, root, &body, false);
            prepare_root(&mut emulator, root, OUTSIDE_RETURN, 0x2000);
            emulator.hybrid_parts().0.hybrid_enable_instruction_counts();
            let mut audio = HeadlessAudio::default();
            physical_tick(&mut emulator, &mut audio); // Real main-thread JSR, outside ISR root.
            assert_eq!(emulator.hybrid_cpu().pc(), target);
            assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
            assert_eq!(emulator.hybrid_bus().interrupt_level(), 4);
            let child_sp = emulator.hybrid_cpu().stack_pointer();
            assert_eq!(actual_return_pc(&emulator, child_sp).unwrap(), root + 6);
            accept_real_hint_at_child(&mut emulator, target);
            let mut clock = clock(revision);
            let scope = clock
                .enter_call_ceiling(ids::LEVEL4_ISR, CallCeiling::DEFAULT)
                .unwrap();
            let run = run_original_timed_until_with_audio(
                &mut emulator,
                &mut clock,
                ReferenceBoundary {
                    pc: target,
                    sp: None,
                    source: Some((HANDLER, HANDLER + 2)),
                    min_instructions: 1,
                    entry_exception_return: true,
                },
                Vec::new(),
                true,
                PhysicalObservers::default(),
            )
            .unwrap();
            assert!(run.root_main_exited);
            assert_eq!(emulator.hybrid_cpu().pc(), target);
            assert_eq!(emulator.hybrid_cpu().stack_pointer(), child_sp);
            assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
            assert_eq!(clock.call_clock.usage().instructions, 1); // Only the ISR's actual RTE.
            assert_eq!(clock.call_clock.usage().dispatch_steps, 1); // No outside child scope.
            assert!(clock.callee_scratch.0.is_empty());
            clock.leave_call_ceiling(scope).unwrap();
        }
    }
}

#[test]
fn caller_jsr_and_bsr_open_one_physical_scope_before_real_irq_and_rte_resume() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &function in CAPPED {
            for bsr in [false, true] {
                let target = function.metadata().address(revision);
                let root = target - 32;
                let body = if bsr {
                    vec![0x6100, u16::try_from(target - (root + 2)).unwrap(), 0x4e75]
                } else {
                    vec![0x4eb9, (target >> 16) as u16, target as u16, 0x4e75]
                };
                let mut emulator = before_hint(revision, function, root, &body, false);
                prepare_root(&mut emulator, root, OUTSIDE_RETURN, 0x2000);
                let mut clock = clock(revision);
                let scope = clock
                    .enter_call_ceiling(
                        ids::WAIT_FOR_NEXT_V_BLANK,
                        CallCeiling::new(1000, 10).unwrap(),
                    )
                    .unwrap();
                let boundary =
                    ReferenceBoundary::returned(OUTSIDE_RETURN, crate::catalogue::STACK_TOP);
                let mut run = ClockRun {
                    exception_base: 0,
                    entry_boundary: Some(boundary),
                    ..ClockRun::default()
                };
                run.begin_reference_counting(&mut emulator, true).unwrap();
                run.capture_entry_invocation(&emulator, &mut clock).unwrap();
                let mut audio = HeadlessAudio::default();
                timed_tick(
                    &mut emulator,
                    &mut clock,
                    &mut run,
                    &mut audio,
                    PhysicalObservers::default(),
                )
                .unwrap();
                assert_eq!(emulator.hybrid_cpu().pc(), target);
                assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
                assert_eq!(emulator.hybrid_bus().interrupt_level(), 4);
                assert_eq!(run.nested_ceilings.len(), 1);
                let invocation = run.nested_ceilings[0].invocation;
                assert_eq!(invocation.function, function);
                assert_eq!(invocation.return_pc, root + if bsr { 4 } else { 6 });
                assert!(invocation.matches_entry(&emulator).unwrap());
                let token = format!("{:?}", run.nested_ceilings[0].scope);
                let before_irq = clock.call_clock.usage();
                let mut entered = false;
                for _ in 0..20 {
                    timed_tick(
                        &mut emulator,
                        &mut clock,
                        &mut run,
                        &mut audio,
                        PhysicalObservers::default(),
                    )
                    .unwrap();
                    entered |= emulator.hybrid_cpu().hybrid_exception_depth() == 1;
                    assert_eq!(run.nested_ceilings.len(), 1);
                    assert_eq!(clock.call_clock.usage().dispatch_steps, 2);
                    assert_eq!(format!("{:?}", run.nested_ceilings[0].scope), token);
                    if entered && emulator.hybrid_cpu().hybrid_exception_depth() == 0 {
                        break;
                    }
                }
                assert!(entered);
                assert_eq!(emulator.hybrid_cpu().pc(), target); // Child opcode has not executed.
                assert!(invocation.matches_entry(&emulator).unwrap());
                assert!(clock.call_clock.usage().ticks > before_irq.ticks);
                assert!(!run.root_main_exited);
                while !boundary.reached(emulator.hybrid_cpu(), run.previous_pc, run.instructions)
                    || !native_resume_ready(&emulator)
                {
                    timed_tick(
                        &mut emulator,
                        &mut clock,
                        &mut run,
                        &mut audio,
                        PhysicalObservers::default(),
                    )
                    .unwrap();
                }
                assert!(run.root_main_exited);
                assert!(run.nested_ceilings.is_empty());
                assert_eq!(clock.call_clock.usage().dispatch_steps, 2);
                run.finish_reference_counting(&mut emulator);
                run.finish_nested_ceilings(&mut clock, true).unwrap();
                clock.leave_call_ceiling(scope).unwrap();
            }
        }
    }
}

#[test]
fn own_rts_to_capped_caller_boundary_still_meters_real_tail_isr_calls() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &function in CAPPED {
            let target = function.metadata().address(revision);
            let root = target - 32;
            let mut emulator = before_hint(revision, function, root, &[0x4e75], true);
            prepare_root(&mut emulator, root, target, 0x2000);
            let mut clock = clock(revision);
            let scope = clock
                .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
                .unwrap();
            let run = run_original_timed(
                &mut emulator,
                &mut clock,
                target,
                crate::catalogue::STACK_TOP,
            )
            .unwrap();
            assert!(run.root_main_exited);
            assert_eq!(emulator.hybrid_cpu().pc(), target);
            assert_eq!(
                emulator.hybrid_cpu().stack_pointer(),
                crate::catalogue::STACK_TOP
            );
            assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
            assert_eq!(clock.call_clock.usage().instructions, 4); // root RTS; ISR JSR/child RTS/RTE.
            assert_eq!(clock.call_clock.usage().dispatch_steps, 2); // Real ISR child still scoped.
            assert!(clock.callee_scratch.0.is_empty());
            clock.leave_call_ceiling(scope).unwrap();
        }
    }
}

#[test]
fn unowned_trap_rte_entry_rejects_even_with_the_same_function_id_in_the_clock() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let function = ids::MAP_ASSIGN_TRANSITION_CHAIN;
        let target = function.metadata().address(revision);
        let root = target - 2;
        let mut emulator = machine(revision, function, root, &[0x4e40], false); // TRAP #0 saves target PC.
        prepare_root(&mut emulator, root, OUTSIDE_RETURN, 0x2700);
        let mut clock = clock(revision);
        let scope = clock
            .enter_call_ceiling(function, CallCeiling::DEFAULT)
            .unwrap();
        assert!(clock.call_clock.contains_scope(function));
        let error = run_original_timed(
            &mut emulator,
            &mut clock,
            OUTSIDE_RETURN,
            crate::catalogue::STACK_TOP,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("unsupported unmetered entry"),
            "{revision:?}: {error}"
        );
        assert_eq!(emulator.hybrid_cpu().pc(), target);
        assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
        assert!(clock.call_clock.check().is_err());
        assert!(clock.leave_call_ceiling(scope).is_err());
        assert!(clock.callee_scratch.0.is_empty());
    }
}

#[test]
fn already_spent_coalesced_call_and_irq_observation_fails_without_opening_a_child() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let function = ids::MAP_ASSIGN_TRANSITION_CHAIN;
        let target = function.metadata().address(revision);
        let root = target - 32;
        let body = [0x4eb9, (target >> 16) as u16, target as u16, 0x4e75];
        let mut emulator = before_hint(revision, function, root, &body, false);
        prepare_root(&mut emulator, root, OUTSIDE_RETURN, 0x2000);
        emulator.hybrid_parts().0.hybrid_enable_instruction_counts();
        let sp = emulator.hybrid_cpu().stack_pointer();
        let master_clock = emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        let mut audio = HeadlessAudio::default();
        physical_tick(&mut emulator, &mut audio);
        let instruction = emulator
            .hybrid_cpu()
            .hybrid_instruction_counts()
            .unwrap()
            .last_instruction
            .unwrap();
        assert!(instruction.completed_successfully);
        assert_eq!(instruction.opcode, 0x4eb9);
        let ticks = 1 + accept_real_hint_at_child(&mut emulator, target);
        let mut clock = clock(revision);
        let scope = clock
            .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        clock
            .call_clock
            .record(ClockUsage {
                ticks,
                instructions: 1,
                master_clocks: emulator.hybrid_bus().cycles.m68k_mclk_cycles - master_clock,
                ..ClockUsage::default()
            })
            .unwrap();
        let spent = clock.call_clock.usage();
        let mut run = ClockRun::default();
        let error = run
            .meter_callee_entry(&emulator, &mut clock, Some(instruction), sp)
            .unwrap_err();
        assert!(error.to_string().contains("no exact owned metering split"));
        assert!(run.nested_ceilings.is_empty());
        assert_eq!(clock.call_clock.usage(), spent);
        assert_eq!(clock.call_clock.usage().dispatch_steps, 1);
        assert!(clock.call_clock.check().is_err());
        assert!(clock.leave_call_ceiling(scope).is_err());
    }
}

#[test]
fn real_irq_call_frame_overwrite_cannot_reuse_an_invocation_by_id_alone() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let function = ids::MAP_ASSIGN_TRANSITION_CHAIN;
        let target = function.metadata().address(revision);
        let root = target - 32;
        let body = [0x4eb9, (target >> 16) as u16, target as u16, 0x4e75];
        let mut emulator = before_hint(revision, function, root, &body, false);
        // Authored handler: MOVE.L #outside_return,6(SP); RTE. It corrupts the
        // real child's JSR return word above its real six-byte IRQ frame.
        let handler = [
            0x2f7c,
            (OUTSIDE_RETURN >> 16) as u16,
            OUTSIDE_RETURN as u16,
            6,
            0x4e73,
        ];
        let rom = emulator
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .unwrap()
            .debug_rom_view();
        for (index, word) in handler.into_iter().enumerate() {
            rom[HANDLER as usize / 2 + index] = word;
        }
        prepare_root(&mut emulator, root, OUTSIDE_RETURN, 0x2000);
        let mut clock = clock(revision);
        let scope = clock
            .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        let error = run_original_timed(
            &mut emulator,
            &mut clock,
            OUTSIDE_RETURN,
            crate::catalogue::STACK_TOP,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("unsupported unmetered entry"),
            "{revision:?}: {error}"
        );
        assert_eq!(emulator.hybrid_cpu().pc(), target);
        assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
        assert_eq!(
            actual_return_pc(&emulator, emulator.hybrid_cpu().stack_pointer()).unwrap(),
            OUTSIDE_RETURN
        );
        assert_eq!(clock.call_clock.usage().dispatch_steps, 2);
        assert!(clock.call_clock.check().is_err());
        assert!(clock.callee_scratch.0.is_empty());
        assert!(clock.leave_call_ceiling(scope).is_err());
    }
}
