use super::*;

fn runtime() -> DeviceRuntime {
    let mut rom = vec![0; 256];
    rom[0x40..0x44].copy_from_slice(&[0x12, 0x34, 0xab, 0xcd]);
    let mut memory = AddressSpace::new(rom, tje_mem::Revision::Rev02);
    memory.write_u16(0xff1234, 0x5678);
    let mut runtime = DeviceRuntime::new(memory, &GenesisEmulatorConfig::default());
    runtime.enable_trace();
    runtime
}

#[test]
fn discarded_rom_and_ram_reads_use_the_real_mutable_bus_latch() {
    let mut runtime = runtime();
    let before_clock = runtime.bus.cycles.m68k_mclk_cycles;
    runtime.discard_bus_word(0x40);
    assert_eq!(runtime.bus.open_bus, 0x1234);
    runtime.discard_bus_byte(0x43);
    assert_eq!(runtime.bus.open_bus, 0xcd34);
    runtime.discard_bus_word(0xff1234);
    assert_eq!(runtime.bus.open_bus, 0x5678);
    runtime.discard_bus_byte(0xff1235);
    assert_eq!(runtime.bus.open_bus, 0x7878);
    assert_eq!(runtime.bus.memory.read_u16(0xff1234), 0x5678);
    assert_eq!(runtime.bus.cycles.m68k_mclk_cycles, before_clock);
    // DeviceTrace remains a device-only observer. RAM/ROM must nevertheless
    // perform the real bus operation, rather than a pure AddressSpace peek.
    assert!(runtime.trace().unwrap().accesses.is_empty());
}

#[test]
fn discarded_device_read_is_single_tagged_and_keeps_vdp_latch_effects() {
    let mut runtime = runtime();
    runtime.write_immediate_word(0xc00004, 0x4000);
    assert!(runtime.bus.vdp.hybrid_checkpoint().control_word_pending);
    runtime.discard_bus_word(0xc00004);
    assert!(!runtime.bus.vdp.hybrid_checkpoint().control_word_pending);
    runtime.discard_bus_byte(0xa10003);
    runtime.read_bus_word(0xa10003);
    let accesses = &runtime.trace().unwrap().accesses;
    assert_eq!(accesses.len(), 4);
    assert!(accesses[0].write);
    assert_eq!(accesses[0].purpose, None);
    assert_eq!(
        (
            accesses[1].address,
            accesses[1].word,
            accesses[1].write,
            accesses[1].purpose
        ),
        (0xc00004, true, false, Some(2))
    );
    assert_eq!(
        (accesses[2].address, accesses[2].word, accesses[2].purpose),
        (0xa10003, false, Some(2))
    );
    assert_eq!(
        (accesses[3].address, accesses[3].word, accesses[3].purpose),
        (0xa10003, true, Some(0))
    );
}
