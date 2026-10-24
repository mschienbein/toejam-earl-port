//! Authored ISA programs test measurement mechanics, not cartridge equivalence.
use super::*;
use crate::fixture::tests::synthetic_host;
use m68000_emu::BusInterface;

fn host(words: &[u16]) -> Host {
    let mut host = synthetic_host(&[0x60fe], &[0x4e71, 0x4e73]);
    host.set_wait_input_poller(std::sync::Arc::new(crate::input::InputScript::default()));
    let address = ids::MOVE_DATA_TO_VRAM.metadata().address(host.revision);
    let rom = host
        .emulator
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .debug_rom_view();
    for (index, &word) in words.iter().enumerate() {
        rom[address as usize / 2 + index] = word;
    }
    host
}
fn encoded(host: &Host) -> Vec<u8> {
    bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
}
fn ceiling() -> CallCeiling {
    CallCeiling {
        ticks: 1_000_000,
        frames: 8,
        reference_instructions: Some(200_000),
    }
}

#[test]
fn only_actual_successful_rts_finishes_and_entire_live_machine_is_restored() {
    // MOVE.L 8(SP),D0; RTS. No caller JSR is executed or claimed.
    let mut host = host(&[0x202f, 0x0008, 0x4e75]);
    host.audio.samples.push([0.25, -0.5]);
    host.measurements.logic_ticks = 123;
    let before = encoded(&host);
    let caller = host.cpu().pc();
    let sp = host.cpu().stack_pointer();
    for count in [0_u32, 3] {
        let record = host
            .measure_original_clocked_fixture(
                ids::MOVE_DATA_TO_VRAM,
                &[0x80000000, count],
                ceiling(),
            )
            .unwrap();
        assert_eq!(record.purpose, "measurement");
        assert!(!record.approves_translation);
        assert!(record.entry_origin.contains("caller JSR not observed"));
        assert_eq!(record.arguments, [0x80000000, count]);
        assert_eq!(record.caller_pc, caller);
        assert_eq!(record.return_pc, caller);
        assert_eq!(record.entry_sp, sp - 12);
        assert_eq!(record.return_sp, sp - 8);
        assert_eq!(record.usage_to_rts.main_thread_instructions, 2);
        assert_eq!(record.usage_to_rts.exception_handler_instructions, 0);
        assert_eq!(record.usage_to_rts.faulted_opcode_attempts, 0);
        assert_eq!(record.successful_rte_returns, 0);
        assert!(record.usage_to_rts.ticks >= 2);
        assert_eq!(before, encoded(&host));
        assert_eq!(host.frame, 0);
        assert_eq!(host.measurements.logic_ticks, 123);
        assert_eq!(host.audio.samples, [[0.25, -0.5]]);
        assert_eq!(host.checks.calls, 0);
    }
}

#[test]
fn real_irq_entries_and_rte_attempt_origin_are_counted_without_masking_them() {
    // MOVE.W #32767,D0; DBRA D0,*; RTS: enough real clocks for VBlank.
    let mut host = host(&[0x303c, 0x7fff, 0x51c8, 0xfffe, 0x4e75]);
    host.context().set_status_register(0x2000);
    let bus = host.emulator.hybrid_bus_mut();
    bus.write_word(0xc00004, 0x8160); // display and VBlank interrupt enabled
    bus.apply_pending_writes();
    let before = encoded(&host);
    let record = host
        .measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], ceiling())
        .unwrap();
    assert_eq!(record.entry_sr & 0x700, 0);
    assert_eq!(record.usage_to_rts.main_thread_instructions, 32770);
    assert!(record.usage_to_rts.exception_handler_instructions > 0);
    assert!(record.usage_to_rts.irq_entries_by_level[6] > 0);
    assert_eq!(
        record.successful_rte_returns,
        record.usage_to_rts.irq_entries_by_level.iter().sum::<u64>()
    );
    assert_eq!(record.usage_to_rts.other_exception_entries, 0);
    assert_eq!(record.usage_to_rts.faulted_opcode_attempts, 0);
    assert!(record.usage_to_rts.completed_video_frames > 0);
    assert_eq!(encoded(&host), before);
    assert_eq!(host.frame, 0);
}

#[test]
fn finite_tick_and_instruction_cutoffs_and_faults_never_return_a_completed_measurement() {
    for limit in [
        CallCeiling {
            ticks: 3,
            frames: 8,
            reference_instructions: Some(100),
        },
        CallCeiling {
            ticks: 100,
            frames: 8,
            reference_instructions: Some(1),
        },
    ] {
        let mut host = host(&[0x60fe]);
        let before = encoded(&host);
        let error = host
            .measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], limit)
            .unwrap_err();
        assert!(error.to_string().contains("ceiling exceeded"));
        assert_eq!(encoded(&host), before);
        assert_eq!(host.checks.calls, 0);
    }
    let mut host = host(&[0x4afc]); // authored ILLEGAL opcode, not a successful RTS
    let before = encoded(&host);
    assert!(
        host.measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], ceiling())
            .is_err()
    );
    assert_eq!(encoded(&host), before);
}

#[test]
fn wrong_mode_root_arity_or_implicit_resource_policy_rejects_before_effects() {
    for mode in [Mode::Check, Mode::Replace] {
        let mut host = host(&[0x4e75]);
        host.mode = mode;
        let before = encoded(&host);
        assert!(
            host.measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], ceiling())
                .is_err()
        );
        assert_eq!(encoded(&host), before);
    }
    let mut host = host(&[0x4e75]);
    let before = encoded(&host);
    assert!(
        host.measure_original_clocked_fixture(ids::GET_CTRL1_INPUT, &[0, 0], ceiling())
            .is_err()
    );
    assert!(
        host.measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0], ceiling())
            .is_err()
    );
    assert!(
        host.measure_original_clocked_fixture(
            ids::MOVE_DATA_TO_VRAM,
            &[0, 0],
            CallCeiling::DEFAULT
        )
        .is_err()
    );
    let scope = host
        .wait_clock
        .call_clock
        .enter(ids::MOVE_DATA_TO_VRAM, ceiling())
        .unwrap();
    assert!(
        host.measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], ceiling())
            .is_err()
    );
    host.wait_clock.call_clock.leave(scope).unwrap();
    assert_eq!(encoded(&host), before);
}

#[test]
fn bad_real_caller_targets_and_unsettled_hardware_are_not_repaired_for_measurement() {
    for pc in [crate::abi::CALL_SENTINEL, 0xff1000, 0x3ffff0, 0x101] {
        let mut host = host(&[0x4e75]);
        let (cpu, bus) = host.emulator.hybrid_parts();
        cpu.set_pc(pc, bus);
        let before = encoded(&host);
        assert!(
            host.measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], ceiling())
                .is_err()
        );
        assert_eq!(encoded(&host), before);
    }
    let mut host = host(&[0x4e75]);
    host.emulator.hybrid_bus_mut().cycles.m68k_wait_cpu_cycles = 1;
    let before = encoded(&host);
    assert!(
        host.measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], ceiling())
            .is_err()
    );
    assert_eq!(encoded(&host), before);
}

#[test]
fn existing_counter_history_and_pause_state_are_not_reset_or_leaked() {
    for enabled in [false, true] {
        let mut host = host(&[0x7005, 0x4e75]);
        host.emulator
            .hybrid_parts()
            .0
            .hybrid_enable_instruction_counts();
        for _ in 0..20 {
            host.tick_live_original_counted(&mut Inputs(Default::default()), true)
                .unwrap();
            if host.cpu().hybrid_instruction_counts().unwrap().main_thread > 0 {
                break;
            }
        }
        host.settle_fixture_boundary(Default::default()).unwrap();
        host.emulator
            .hybrid_parts()
            .0
            .hybrid_set_instruction_counting(enabled);
        let counts = *host.cpu().hybrid_instruction_counts().unwrap();
        assert!(counts.main_thread > 0);
        let record = host
            .measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], ceiling())
            .unwrap();
        assert_eq!(record.entry.main_thread_instructions, counts.main_thread);
        assert_eq!(record.usage_to_rts.main_thread_instructions, 2);
        let after = host.cpu().hybrid_instruction_counts().unwrap();
        assert_eq!(after.main_thread, counts.main_thread);
        assert_eq!(after.enabled(), enabled);
    }
}

#[path = "rts_frame_tests.rs"]
mod rts_frame_tests;
