use genesis_config::{GenesisController, GenesisEmulatorConfig, GenesisInputs, GenesisJoypadState};
#[cfg(feature = "game-adapter")]
use tje_game::{
    Context, Platform,
    execution::{WaitCondition, WaitRequest},
    ids,
};
use tje_mem::{AddressSpace, Bus, Revision};
use tje_native_platform::{ClockedDevices, DeviceRuntime, DeviceWrite, Registers};
#[cfg(feature = "game-adapter")]
use tje_native_platform::{InterruptHandlers, NativePlatform, StartError};

fn memory() -> AddressSpace {
    AddressSpace::new(vec![0; 256], Revision::Rev02)
}
fn devices() -> DeviceRuntime {
    DeviceRuntime::new(memory(), &GenesisEmulatorConfig::default())
}
#[cfg(feature = "game-adapter")]
fn context() -> NativePlatform {
    NativePlatform::new(
        memory(),
        &[],
        InterruptHandlers::default(),
        &GenesisEmulatorConfig::default(),
    )
    .unwrap()
}

#[test]
fn stores_publish_after_instruction_and_stalls_remain_explicit() {
    let mut runtime = devices();
    runtime.begin_instruction(0, 0);
    runtime.write_bus_word(0xff1234, 0xabcd);
    assert_eq!(runtime.memory().read_u16(0xff1234), 0);
    let step = runtime.finish_instruction(0x100, 4);
    assert_eq!(step.master_clocks, 28);
    assert_eq!(runtime.memory().read_u16(0xe01234), 0xabcd);
    assert_eq!(runtime.clock(), 28);
    runtime.write_immediate_word(0xa11200, 0x100);
    runtime.write_immediate_word(0xa11100, 0x100);
    runtime.begin_instruction(0, 0);
    runtime.write_bus_word(0xa00000, 0xabcd);
    runtime.finish_instruction(0x100, 4);
    assert_eq!(runtime.audio_ram()[0], 0xab);
    assert!(runtime.wait_pending());
    runtime.begin_instruction(0, 0);
    assert!(runtime.take_wait_step(0x100).is_some());
    runtime.begin_instruction(0, 0);
    runtime.finish_instruction(0x100, 4);
}

#[test]
fn z80_ownership_width_and_reset_use_real_chip_semantics() {
    let mut runtime = devices();
    runtime.write_immediate_word(0xa11100, 0x100);
    runtime.write_immediate_byte(0xa00010, 0x42);
    assert_eq!(runtime.audio_ram()[0x10], 0, "reset prevents BUSACK");
    runtime.write_immediate_word(0xa11200, 0x100);
    runtime.write_immediate_word(0xa00010, 0xabcd);
    assert_eq!(runtime.read_bus_word(0xa02010), 0xabab);
    runtime.write_immediate_byte(0xa04000, 0x2a);
    runtime.write_immediate_byte(0xa04001, 0x80);
    assert_ne!(runtime.read_bus_byte(0xa04000) & 0x80, 0);
    runtime.advance_master_clocks(2000);
    assert_eq!(runtime.read_bus_byte(0xa04000) & 0x80, 0);
    runtime.write_immediate_word(0xa11200, 0);
    runtime.write_immediate_word(0xa11200, 0x100);
    assert_eq!(
        runtime.read_bus_byte(0xa04000) & 0x80,
        0,
        "Z80 reset also resets YM2612"
    );
}

#[test]
fn controller_th_protocol_uses_retained_input_model() {
    let mut runtime = devices();
    runtime.set_inputs(GenesisInputs {
        p1: GenesisController::ThreeButton(GenesisJoypadState {
            a: true,
            b: true,
            start: true,
            up: true,
            ..GenesisJoypadState::default()
        }),
        p2: GenesisController::None,
    });
    runtime.write_immediate_byte(0xa10009, 0x40);
    runtime.write_immediate_byte(0xa10003, 0x40);
    let high = runtime.read_bus_byte(0xa10003);
    assert_eq!(high & 0x11, 0, "TH-high samples Up and B");
    runtime.write_immediate_byte(0xa10003, 0);
    let low = runtime.read_bus_byte(0xa10002);
    assert_eq!(
        low & 0x3d,
        0,
        "TH-low samples Up, A, Start and signature bits"
    );
    assert_eq!(
        runtime.read_bus_word(0xa10004),
        0xffff,
        "unplugged controller floats high"
    );
}

#[test]
fn vdp_byte_duplication_word_order_and_psg_lane_are_preserved() {
    let mut runtime = devices();
    runtime.enable_trace();
    runtime.write_immediate_word(0xc00004, 0x8f02);
    runtime.write_immediate_word(0xc00004, 0x4000);
    runtime.write_immediate_word(0xc00004, 0);
    runtime.write_immediate_byte(0xc00000, 0x5a);
    runtime.write_immediate_byte(0xc00010, 0x9f);
    runtime.write_immediate_byte(0xc00011, 0x9f);
    runtime.advance_master_clocks(1000);
    assert_eq!(&runtime.vdp().hybrid_memories().0[..2], &[0x5a, 0x5a]);
    let writes = &runtime.trace().unwrap().writes;
    assert_eq!(writes.len(), 5);
    assert_eq!(
        writes[3].write,
        DeviceWrite::Vdp {
            port: 0,
            value: 0x5a5a
        }
    );
    assert_eq!(writes[4].write, DeviceWrite::Psg { value: 0x9f });
}

fn dma(runtime: &mut DeviceRuntime, source: u32) {
    for value in [
        0x8114,
        0x8f02,
        0x9302,
        0x9400,
        0x9500 | ((source >> 1) as u16 & 0xff),
        0x9600 | ((source >> 9) as u16 & 0xff),
        0x9700 | ((source >> 17) as u16 & 0x7f),
        0x4000,
        0x0080,
    ] {
        runtime.write_immediate_word(0xc00004, value);
    }
    runtime.advance_master_clocks(1000);
}

#[test]
fn dma_reads_both_tje_ram_and_runtime_rom() {
    let mut runtime = devices();
    runtime.memory_mut().write_u32(0xff0000, 0x1234abcd);
    dma(&mut runtime, 0xff0000);
    assert_eq!(
        &runtime.vdp().hybrid_memories().0[..4],
        &[0x12, 0x34, 0xab, 0xcd]
    );
    let mut rom = vec![0; 256];
    rom[16..20].copy_from_slice(&[0xa1, 0xb2, 0xc3, 0xd4]);
    let mut runtime = DeviceRuntime::new(
        AddressSpace::new(rom, Revision::Rev02),
        &GenesisEmulatorConfig::default(),
    );
    dma(&mut runtime, 16);
    assert_eq!(
        &runtime.vdp().hybrid_memories().0[..4],
        &[0xa1, 0xb2, 0xc3, 0xd4]
    );
}

#[cfg(feature = "game-adapter")]
#[test]
fn native_closure_rejects_missing_translations_without_rom_fallback() {
    let result = NativePlatform::new(
        memory(),
        &[ids::GAME_PROGRAM],
        InterruptHandlers::default(),
        &GenesisEmulatorConfig::default(),
    );
    let Err(StartError::MissingTranslations(missing)) = result else {
        panic!("untranslated native root accepted")
    };
    assert!(!missing.is_empty());
    let mut context = context();
    assert!(matches!(
        context.call_original(ids::GAME_PROGRAM, &[]),
        Err(tje_game::Error::Untranslated(_))
    ));
}

#[cfg(feature = "game-adapter")]
#[test]
fn waits_progress_devices_and_fail_closed_without_a_satisfied_predicate() {
    let mut context = context();
    context.write_device_u16(0xa11200, 0x100);
    context.write_device_u16(0xa11100, 0x100);
    context.write_ym2612(0, 0x2a);
    context.write_ym2612(1, 0x80);
    let clock = context.devices().clock();
    context.wait_until(WaitRequest::ym2612_ready()).unwrap();
    assert!(context.devices().clock() > clock);
    let request = WaitRequest {
        condition: WaitCondition::RamWordChanged {
            address: 0xff0100,
            previous: 0,
        },
        max_ticks: 3,
    };
    assert!(context.wait_until(request).is_err());
    assert!(
        context
            .wait_until(WaitRequest {
                condition: WaitCondition::Interrupt,
                max_ticks: 2
            })
            .is_err()
    );
}

#[cfg(feature = "game-adapter")]
#[test]
fn fill_join_uses_real_devices_without_polling_status_or_normalizing_other_dma() {
    let mut context = context();
    context.devices_mut().enable_trace();
    let before = context.devices().clock();
    let error = context
        .wait_until(WaitRequest::vdp_fill_complete())
        .unwrap_err();
    assert!(error.to_string().contains("VRAM-fill DMA mode"), "{error}");
    assert_eq!(context.devices().clock(), before);
    assert!(context.devices().trace().unwrap().accesses.is_empty());

    for value in [
        0x8114, 0x8f01, 0x9300, 0x9402, 0x9500, 0x9600, 0x9780, 0x4000, 0x0080,
    ] {
        context.write_device_u16(0xc00004, value);
    }
    context.write_device_u16(0xc00000, 0xabcd);
    assert!(context.devices().vdp().timing_dma_active());
    context.devices_mut().clear_trace();
    let before = context.devices().clock();
    assert!(
        context
            .wait_until(WaitRequest {
                max_ticks: 1,
                ..WaitRequest::vdp_fill_complete()
            })
            .is_err()
    );
    assert!(context.devices().clock() > before);
    assert!(context.devices().vdp().timing_dma_active());
    context
        .wait_until(WaitRequest::vdp_fill_complete())
        .unwrap();
    assert!(!context.devices().vdp().timing_dma_active());
    assert_eq!(context.devices().vdp().hybrid_registers().dma_length, 0);
    assert!(context.devices().trace().unwrap().accesses.is_empty());
    let completed = context.devices().clock();
    context
        .wait_until(WaitRequest::vdp_fill_complete())
        .unwrap();
    assert_eq!(context.devices().clock(), completed);
    assert!(context.devices().trace().unwrap().accesses.is_empty());
}

#[test]
fn captures_round_trip_every_mutable_device_and_clock_phase() {
    let mut runtime = devices();
    runtime.write_immediate_word(0xa11200, 0x100);
    runtime.write_immediate_word(0xa11100, 0x100);
    runtime.write_immediate_byte(0xa00000, 0x76);
    runtime.begin_instruction(0, 0);
    runtime.write_bus_word(0xff0040, 0x4a1b);
    runtime.finish_instruction(0x100, 13);
    let state = runtime.capture_state(Registers::default());
    let restored = DeviceRuntime::from_state(memory(), state).unwrap();
    assert_eq!(runtime.memory().ram(), restored.memory().ram());
    assert_eq!(runtime.audio_ram(), restored.audio_ram());
    assert!(runtime.vdp().hybrid_state_eq(restored.vdp()));
    let config = bincode::config::standard();
    assert_eq!(
        bincode::encode_to_vec(runtime.ym2612(), config).unwrap(),
        bincode::encode_to_vec(restored.ym2612(), config).unwrap()
    );
    assert_eq!(
        bincode::encode_to_vec(runtime.z80(), config).unwrap(),
        bincode::encode_to_vec(restored.z80(), config).unwrap()
    );
    assert_eq!(
        bincode::encode_to_vec(runtime.cycles(), config).unwrap(),
        bincode::encode_to_vec(restored.cycles(), config).unwrap()
    );
}

#[cfg(feature = "game-adapter")]
#[test]
fn native_context_preserves_real_frames_residue_and_banked_stacks() {
    use tje_game::{
        call_frame::{FrameLayout, FrameState, LocalFrame},
        exits::{StackBanks, restore_exception},
    };
    let mut context = context();
    context
        .set_stack_banks(StackBanks {
            user: 0xff7800,
            supervisor: 0xff7e00,
        })
        .unwrap();
    context.set_status_register(0x2000);
    context.set_frame_state(FrameState::new(0xff7e00)).unwrap();
    context.set_data_register(2, 0x1234abcd);
    context.set_address_register(6, 0xff7000);
    context.write_u32(0xff7e04, 0x11223344);
    context.write_u32(0xff7df8, 0xaabbccdd);
    let frame = LocalFrame::enter(&mut context, FrameLayout::link(8, 4, 0)).unwrap();
    assert_eq!(frame.argument_slot(0).read(&context), 0x11223344);
    assert_eq!(
        context.read_u32(0xff7df8),
        0xaabbccdd,
        "unwritten local bytes retain residue"
    );
    context.set_data_register(2, 7);
    frame.leave(&mut context).unwrap();
    assert_eq!(context.data_register(2), 0x1234abcd);
    assert_eq!(context.address_register(6), 0xff7000);
    assert_eq!(context.stack_pointer().unwrap(), 0xff7e00);
    context.set_stack_pointer(0xff7dfa).unwrap();
    context.write_u16(0xff7dfa, 0);
    context.write_u32(0xff7dfc, 0x1234);
    let exception = restore_exception(&mut context).unwrap();
    assert_eq!(exception.program_counter, 0x1234);
    assert_eq!(context.stack_pointer().unwrap(), 0xff7800);
    assert_eq!(context.stack_banks().unwrap().supervisor, 0xff7e00);
}

#[cfg(feature = "game-adapter")]
#[test]
fn prepared_native_rts_rejects_odd_return_pc_and_entry_stack() {
    use tje_game::{Error, call_frame::FrameError};
    let mut context = NativePlatform::new(
        memory(),
        &[ids::DISABLE_ALL_INTERRUPTS],
        InterruptHandlers::default(),
        &GenesisEmulatorConfig::default(),
    )
    .unwrap();
    context.set_stack_pointer(0xFF7E00).unwrap();
    context.write_u32(0xFF7E00, 0x301);
    assert!(matches!(
        context.invoke(ids::DISABLE_ALL_INTERRUPTS, &[]),
        Err(Error::Frame(FrameError::UnalignedReturnPc(0x301)))
    ));
    assert_eq!(context.stack_pointer().unwrap(), 0xFF7E00);
    context.set_stack_pointer(0xFF7E01).unwrap();
    assert!(matches!(
        context.invoke(ids::DISABLE_ALL_INTERRUPTS, &[]),
        Err(Error::Frame(FrameError::UnalignedStack(0xFF7E01)))
    ));
}

#[test]
fn frame_audio_capacity_and_device_only_interrupt_lines_are_real() {
    let mut runtime = devices();
    runtime.set_audio_sample_rate(192_000);
    runtime.write_immediate_word(0xc00004, 0x8124);
    runtime.begin_instruction(0x4e71, 56);
    runtime.finish_instruction(0x100, 4);
    runtime.advance_master_clocks(3420 * 225);
    assert_eq!(runtime.pending_interrupt(), 6);
    runtime.acknowledge_interrupt();
    assert_eq!(runtime.pending_interrupt(), 0);
    assert!((runtime.target_fps() - 59.922743).abs() < 0.000001);
    assert!(!runtime.audio_samples().is_empty());
    runtime.enable_trace();
    runtime.disable_trace();
    assert!(runtime.trace().is_none());
    runtime.set_reset_asserted(true);
    assert!(runtime.reset_asserted());
    runtime.reset();
    assert!(!runtime.reset_asserted());
    assert_eq!(runtime.clock(), 0);
    assert!(runtime.audio_samples().is_empty());
}

#[test]
fn reference_instruction_journal_preserves_purpose_issue_order_and_origin() {
    use genesis_core::{GenesisBus, native::CpuAccess};
    use jgenesis_common::frontend::TimingMode;
    use m68000_emu::{BusInterface, ReadPurpose};
    let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
    bus.native_cpu_accesses = Some(Vec::new());
    bus.native_ram_writes = Some(Vec::new());
    assert_eq!(bus.read_word_tagged(0x01000000, ReadPurpose::Prefetch), 0);
    bus.write_byte(0x01ff1234, 0x7a);
    assert_eq!(bus.read_byte_tagged(0x01ff1234, ReadPurpose::Discarded), 0);
    bus.write_word(0x01ff1236, 0xbeef);
    assert_eq!(bus.read_word_tagged(0x01ff1236, ReadPurpose::Semantic), 0);
    bus.acknowledge_interrupt(6);
    let expected = vec![
        CpuAccess::Read {
            address: 0,
            word: true,
            value: 0,
            purpose: 1,
        },
        CpuAccess::Write {
            address: 0xff1234,
            word: false,
            value: 0x7a,
        },
        CpuAccess::Read {
            address: 0xff1234,
            word: false,
            value: 0,
            purpose: 2,
        },
        CpuAccess::Write {
            address: 0xff1236,
            word: true,
            value: 0xbeef,
        },
        CpuAccess::Read {
            address: 0xff1236,
            word: true,
            value: 0,
            purpose: 0,
        },
        CpuAccess::AcknowledgeInterrupt { level: 6 },
    ];
    assert_eq!(bus.native_cpu_accesses.as_ref().unwrap(), &expected);
    bus.apply_pending_writes();
    assert_eq!(bus.memory.read_main_ram_word(0xff1234), 0x7a00);
    assert_eq!(bus.memory.read_main_ram_word(0xff1236), 0xbeef);
    assert_eq!(
        bus.native_ram_writes.as_ref().unwrap(),
        &[
            genesis_core::native::RamWrite::new(0xff1234, false, 0x7a),
            genesis_core::native::RamWrite::new(0xff1236, true, 0xbeef),
        ]
    );
    z80_emu::BusInterface::read_memory(&mut bus, 0x8000);
    assert_eq!(
        bus.native_cpu_accesses.as_ref().unwrap(),
        &expected,
        "neither commit nor Z80 bank reads are CPU-issued events"
    );
    let mut cpu = m68000_emu::M68000::default();
    cpu.set_data_registers([7; 8]);
    cpu.set_address_registers([9; 7], 0xff7000, 0xff7e00);
    let words = cpu.hybrid_state_words();
    assert_eq!(&words[..8], &[7; 8]);
    assert_eq!(&words[8..15], &[9; 7]);
    assert_eq!(words[15], 0xff7000);
    assert_eq!(words[16], 0xff7e00);
    assert_eq!(words[18], u32::from(cpu.status_register()));
    assert_eq!(words[20], 0);
}

#[test]
fn actual_ram_commit_trace_covers_cpu_mirrors_and_running_z80_writes() {
    use tje_native_platform::RamWrite;
    let mut runtime = devices();
    runtime.enable_ram_write_trace();
    runtime.begin_instruction(0x4e71, 56);
    runtime.write_bus_word(0xe01235, 0xabcd);
    assert!(runtime.ram_write_trace().unwrap().is_empty());
    runtime.finish_instruction(0x100, 4);
    assert_eq!(
        runtime.ram_write_trace().unwrap(),
        &[RamWrite::new(0xff1234, true, 0xabcd)]
    );
    runtime.clear_ram_write_trace();
    runtime.write_immediate_word(0xa11200, 0x100);
    runtime.write_immediate_word(0xa11100, 0x100);
    // Authored ROM-free Z80 fixture: LD A,$7A; LD ($8001),A; HALT.
    for (offset, value) in [0x3e, 0x7a, 0x32, 0x01, 0x80, 0x76].into_iter().enumerate() {
        runtime.write_immediate_byte(0xa00000 + offset as u32, value);
    }
    let bank = 0xff0000_u32 >> 15;
    for bit in 0..9 {
        runtime.write_immediate_byte(0xa06000, ((bank >> bit) & 1) as u8);
    }
    runtime.write_immediate_word(0xa11100, 0);
    runtime.advance_master_clocks(1000);
    assert_eq!(runtime.memory().read_u8(0xff0001), 0x7a);
    assert_eq!(
        runtime.ram_write_trace().unwrap(),
        &[RamWrite::new(0xff0001, false, 0x7a)]
    );
    runtime.disable_ram_write_trace();
    assert!(runtime.ram_write_trace().is_none());
}
