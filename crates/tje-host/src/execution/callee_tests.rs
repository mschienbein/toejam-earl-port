//! Authored CPU fixtures only: no game body or commutation approval is exercised.
use super::*;
use crate::{allocation_counter::ALLOCATIONS, exits::ReferenceBoundary};
use genesis_config::GenesisEmulatorConfig;
use m68000_emu::BusInterface;
use tje_game::{execution::CallCeiling, ids};
use tje_rom::Revision;

#[derive(Debug, Clone, Copy)]
enum EntryPath {
    Call,
    Jump,
    Branch,
    ConditionalBranch,
    Dbcc,
    Fallthrough,
}

fn driver(revision: Revision, path: EntryPath, self_loop: bool) -> GenesisEmulator {
    // Only the committed entry address is used. Every byte is authored here.
    let target = ids::MAP_ASSIGN_TRANSITION_CHAIN
        .metadata()
        .address(revision);
    let wrapper = target
        - if matches!(path, EntryPath::Fallthrough) {
            2
        } else {
            32
        };
    let displacement = u16::try_from(target - (wrapper + 2)).unwrap();
    let wrapper_words = match path {
        EntryPath::Call => [0x4eb9, (target >> 16) as u16, target as u16, 0x4e75],
        EntryPath::Jump => [0x4ef9, (target >> 16) as u16, target as u16, 0x4e75],
        EntryPath::Branch => [0x6000, displacement, 0x4e71, 0x4e71],
        EntryPath::ConditionalBranch => [0x6600, displacement, 0x4e71, 0x4e71],
        EntryPath::Dbcc => [0x51c8, displacement, 0x4e71, 0x4e71],
        EntryPath::Fallthrough => [0x4e71; 4],
    };
    let mut rom = vec![0; (target as usize + 16).next_power_of_two().max(0x800)];
    rom[..4].copy_from_slice(&crate::catalogue::STACK_TOP.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    rom[30 * 4..31 * 4].copy_from_slice(&0x400_u32.to_be_bytes());
    for (address, words) in [
        (
            0x100,
            &[0x4eb9, (wrapper >> 16) as u16, wrapper as u16, 0x60f8][..],
        ),
        (wrapper, &wrapper_words[..]),
        (PARK_PC, &[0x60fe, 0x4e71][..]),
        (
            0x400,
            &[0x4eb9, (target >> 16) as u16, target as u16, 0x4e73][..],
        ),
        (target, &[if self_loop { 0x60fe } else { 0x4e75 }][..]),
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
    let cpu = emulator.hybrid_parts().0;
    cpu.set_status_register(0x2700); // BNE taken; IRQs masked for the call driver.
    let mut registers = cpu.data_registers();
    registers[0] = 1; // DBF decrements to zero and takes its actual branch.
    cpu.set_data_registers(registers);
    emulator
}

fn clock(revision: Revision) -> WaitClock {
    WaitClock {
        revision: Some(revision),
        poller: Some(Arc::new(InputScript::default())),
        ..WaitClock::default()
    }
}

fn original_interval(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    samples: Vec<[f64; 2]>,
) -> Vec<[f64; 2]> {
    // Repeated diagnostic entry setup is outside the emulated call interval.
    let (cpu, bus) = emulator.hybrid_parts();
    cpu.set_pc(0x100, bus);
    let sp = cpu.stack_pointer();
    let before = clock.call_clock.usage().instructions;
    let root = clock
        .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
        .unwrap();
    let run = run_original_timed_until_with_audio(
        emulator,
        clock,
        ReferenceBoundary::returned(0x106, sp),
        samples,
        false,
        crate::execution::PhysicalObservers::default(),
    );
    let cleanup = clock.leave_call_ceiling(root);
    let run = run.unwrap();
    cleanup.unwrap();
    assert_eq!(clock.call_clock.usage().instructions - before, 4);
    assert_eq!(emulator.hybrid_cpu().stack_pointer(), sp);
    assert!(clock.callee_scratch.0.is_empty());
    run.audio
}

fn wait_interval(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    samples: Vec<[f64; 2]>,
) -> Vec<[f64; 2]> {
    let root = clock
        .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
        .unwrap();
    let run = run_wait_with_audio(
        emulator,
        clock,
        WaitRequest::interrupt(),
        samples,
        false,
        crate::execution::PhysicalObservers::default(),
    );
    let cleanup = clock.leave_call_ceiling(root);
    let run = run.unwrap();
    cleanup.unwrap();
    assert!(clock.callee_scratch.0.is_empty());
    run.audio
}

#[test]
fn every_unmetered_entry_path_fails_before_the_measured_target_executes() {
    prepare_operational_callee_indexes().unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let target = ids::MAP_ASSIGN_TRANSITION_CHAIN
            .metadata()
            .address(revision);
        for path in [
            EntryPath::Jump,
            EntryPath::Branch,
            EntryPath::ConditionalBranch,
            EntryPath::Dbcc,
            EntryPath::Fallthrough,
        ] {
            let mut emulator = driver(revision, path, false);
            let original_sp = emulator.hybrid_cpu().stack_pointer();
            let mut clock = clock(revision);
            let root = clock
                .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
                .unwrap();
            let error =
                run_original_timed(&mut emulator, &mut clock, 0x106, original_sp).unwrap_err();
            assert!(
                error.to_string().contains("unsupported unmetered entry"),
                "{revision:?} {path:?}: {error}"
            );
            assert_eq!(emulator.hybrid_cpu().pc(), target);
            assert_eq!(emulator.hybrid_cpu().stack_pointer(), original_sp - 4);
            assert_eq!(clock.call_clock.usage().instructions, 2);
            assert!(clock.callee_scratch.0.is_empty());
            assert!(clock.call_clock.check().is_err());
            assert!(clock.leave_call_ceiling(root).is_err());
        }
    }
}

#[test]
fn already_metered_self_entry_keeps_the_ancestor_limit_without_renewal() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut emulator = driver(revision, EntryPath::Call, true);
        let sp = emulator.hybrid_cpu().stack_pointer();
        let mut clock = clock(revision);
        let root = clock
            .enter_call_ceiling(
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallCeiling::new(1000, 10).unwrap(),
            )
            .unwrap();
        let error = run_original_timed(&mut emulator, &mut clock, 0x106, sp).unwrap_err();
        assert!(
            matches!(
                error,
                HostError::Clock(tje_game::execution::CallClockError::CeilingExceeded {
                    resource: tje_game::execution::ClockResource::Ticks,
                    ..
                })
            ),
            "{error}"
        );
        assert_eq!(clock.call_clock.usage().dispatch_steps, 2);
        assert!(clock.callee_scratch.0.is_empty());
        assert!(clock.callee_scratch.0.capacity() > 0);
        assert!(clock.leave_call_ceiling(root).is_err());
        assert!(!clock.call_clock.active());
    }
}

#[test]
fn warmed_timed_original_and_irq_callee_storage_allocates_zero_and_survives_errors() {
    prepare_operational_callee_indexes().unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut emulator = driver(revision, EntryPath::Call, false);
        let mut clock = clock(revision);
        let mut samples = Vec::with_capacity(4096);
        for _ in 0..64 {
            samples = original_interval(&mut emulator, &mut clock, samples);
            samples.clear();
        }
        emulator.hybrid_bus_mut().write_word(0xc00004, 0x8120);
        emulator.hybrid_bus_mut().apply_pending_writes();
        emulator.hybrid_parts().0.set_status_register(0x2000);
        for _ in 0..2 {
            samples = wait_interval(&mut emulator, &mut clock, samples);
            samples.clear();
        }
        let capacity = clock.callee_scratch.0.capacity();
        let pointer = clock.callee_scratch.0.as_ptr();
        assert!(capacity > 0);
        assert_eq!(clock.clone().callee_scratch.0.capacity(), 0);
        let mut moved_clock = clock;
        emulator.hybrid_parts().0.set_status_register(0x2700);
        ALLOCATIONS.with(|counter| counter.set((true, 0, 0)));
        for _ in 0..1000 {
            samples = original_interval(&mut emulator, &mut moved_clock, samples);
            samples.clear();
        }
        emulator.hybrid_parts().0.set_status_register(0x2000);
        for _ in 0..2 {
            samples = wait_interval(&mut emulator, &mut moved_clock, samples);
            samples.clear();
        }
        let (_, allocations, bytes) = ALLOCATIONS.with(|counter| {
            let result = counter.get();
            counter.set((false, result.1, result.2));
            result
        });
        assert_eq!((allocations, bytes), (0, 0), "{revision:?}");
        assert_eq!(moved_clock.callee_scratch.0.capacity(), capacity);
        assert_eq!(moved_clock.callee_scratch.0.as_ptr(), pointer);
        println!(
            "TIMED_ORIGINAL_ALLOCATIONS {revision:?}: calls=1000 IRQ_waits=2 allocations={allocations} bytes={bytes} retained_capacity={capacity}"
        );

        // Both physical runners return the same empty storage before any error
        // propagation. No CPU return, readiness or deadline is repaired.
        let root = moved_clock
            .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        let error = run_wait_with_audio(
            &mut emulator,
            &mut moved_clock,
            WaitRequest::new(WaitCondition::RamWordChanged {
                address: 0xff9001,
                previous: 0,
            }),
            samples,
            false,
            crate::execution::PhysicalObservers::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("aligned RAM word"));
        moved_clock.leave_call_ceiling(root).unwrap();
        assert_eq!(moved_clock.callee_scratch.0.as_ptr(), pointer);
        assert_eq!(moved_clock.callee_scratch.0.capacity(), capacity);
        assert!(moved_clock.callee_scratch.0.is_empty());
        let (cpu, bus) = emulator.hybrid_parts();
        cpu.set_status_register(0x2700);
        cpu.set_pc(0x100, bus);
        let sp = cpu.stack_pointer();
        let root = moved_clock
            .enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::new(1, 10).unwrap())
            .unwrap();
        let error = run_original_timed(&mut emulator, &mut moved_clock, 0x106, sp).unwrap_err();
        assert!(matches!(error, HostError::Clock(_)), "{error}");
        assert!(moved_clock.leave_call_ceiling(root).is_err());
        assert_eq!(moved_clock.callee_scratch.0.as_ptr(), pointer);
        assert_eq!(moved_clock.callee_scratch.0.capacity(), capacity);
        assert!(moved_clock.callee_scratch.0.is_empty());
    }
}
